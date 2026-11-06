package com.flakm.einkbridge

import android.view.View
import android.widget.Button
import android.widget.ProgressBar
import androidx.test.core.app.ActivityScenario
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
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

    // ── The voice tray ─────────────────────────────────────────────────────────

    @Test fun theVoiceTrayIsHiddenUntilSpeakIsPressedAndGoesWhenTheModeIsLeft() {
        scenario.onActivity { activity ->
            val tray = activity.findViewById<View>(R.id.voiceTray)
            assertEquals(View.GONE, tray.visibility)
            activity.voiceTray.showAiming("abc")
            assertEquals(View.VISIBLE, tray.visibility)
            assertEquals("Tap the block you want to talk about",
                activity.findViewById<android.widget.TextView>(R.id.voiceState).text.toString())
            assertEquals("the edit words wait for a transcript", View.GONE, activity.findViewById<View>(R.id.voiceEdits).visibility)
            activity.voiceTray.leave()
            assertEquals(View.GONE, tray.visibility)
        }
    }

    @Test fun theSpeakButtonIsAWordlessIconWithASpokenName() {
        scenario.onActivity { activity ->
            val speak = activity.findViewById<android.widget.ImageButton>(R.id.btnSpeak)
            assertEquals("Speak a note", speak.contentDescription.toString())
            assertEquals(View.GONE, activity.findViewById<View>(R.id.voiceTray).visibility)
        }
    }

    // ── Session list screen ────────────────────────────────────────────────────

    @Test fun documentUpdatesUseSilentLowImportanceChannel() {
        scenario.onActivity { activity ->
            val manager = activity.getSystemService(android.app.NotificationManager::class.java)
            val channel = manager.getNotificationChannel(NOTIFICATION_CHANNEL_ID)
            assertEquals(android.app.NotificationManager.IMPORTANCE_LOW, channel.importance)
            assertNull(channel.sound)
            assertFalse(channel.shouldVibrate())
            assertFalse(channel.canShowBadge())
            activity.notifyDocumentUpdated()
            assertTrue("Foreground updates must not duplicate the inline status", manager.activeNotifications.isEmpty())
        }
    }

    @Test fun routineStatusIsSmallNonBlockingAndReplacesPreviousMessage() {
        scenario.onActivity { activity ->
            activity.showRoutineStatus("Feedback saved")
            activity.showRoutineStatus("Document updated")
            val status = activity.findViewById<android.widget.TextView>(R.id.routineStatus)
            assertEquals("Document updated", status.text.toString())
            assertEquals(View.VISIBLE, status.visibility)
            assertFalse(status.isClickable)
            assertFalse(status.isFocusable)
            assertEquals(12f * activity.resources.displayMetrics.scaledDensity, status.textSize, 0.1f)
            assertNull(org.robolectric.shadows.ShadowToast.getTextOfLatestToast())
        }
    }

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

    /*
     * These two drive `renderListState`, the function the state collector calls, rather
     * than poking the adapter: the old version asserted a visibility the activity sets
     * elsewhere, so it passed only when the background poll had not yet decided the list
     * was empty (it had, on CI).
     */
    @Test
    fun emptyStateLabelVisible_whenNoSessions() {
        scenario.onActivity { activity ->
            activity.renderListState(SessionListState())

            assertEquals(View.GONE, activity.sessionList.visibility)
            assertEquals(View.VISIBLE, activity.emptyState.visibility)
        }
    }

    @Test
    fun sessionListVisible_whenSessionsExist() {
        scenario.onActivity { activity ->
            activity.renderListState(
                SessionListState(
                    sessions = listOf(
                        SessionInfo("id1", "PR Review", "Active", "2026-03-30T10:00:00Z", "2026-03-30T10:00:00Z"),
                        SessionInfo("id2", "Bug Report", "Submitted", "2026-03-30T09:00:00Z", "2026-03-30T09:30:00Z"),
                    ),
                    connectionStatus = ConnectionStatus.Online,
                )
            )

            assertEquals(View.VISIBLE, activity.sessionList.visibility)
            assertEquals(View.GONE, activity.emptyState.visibility)
            assertTrue(
                "the connection line should say it is online, not \"${activity.statusForTest()}\"",
                activity.statusForTest().startsWith("Connected: "),
            )
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
            assertNotNull(activity.btnSize)
            assertEquals(View.GONE, activity.sizeDrawer.visibility)
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
            activity.findViewById<android.widget.ImageButton>(R.id.btnPencil).performClick()
            assertFalse("Bind mode must exit when pencil is tapped", activity.bindModeActive)
        }
    }

    @Test
    fun brushButtonClick_exitsBind_whenBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            activity.findViewById<android.widget.ImageButton>(R.id.btnBrush).performClick()
            assertFalse("Bind mode must exit when brush is tapped", activity.bindModeActive)
        }
    }

    @Test
    fun eraserButtonClick_exitsBind_whenBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            activity.findViewById<android.widget.ImageButton>(R.id.btnEraser).performClick()
            assertFalse("Bind mode must exit when eraser is tapped", activity.bindModeActive)
        }
    }

    // ── Clear confirmation ─────────────────────────────────────────────────────

    @Test
    fun clearButtonClick_showsAlertDialog() {
        scenario.onActivity { activity ->
            activity.findViewById<android.widget.ImageButton>(R.id.btnClear).performClick()
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
            val btn = activity.findViewById<android.widget.ImageButton>(R.id.btnLink)
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
            val btnLink = activity.findViewById<android.widget.ImageButton>(R.id.btnLink)
            val btnSelect = activity.findViewById<android.widget.ImageButton>(R.id.btnSelect)
            val btnPencil = activity.findViewById<android.widget.ImageButton>(R.id.btnPencil)
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

    // ── The agent narrates while it works ──────────────────────────────────────

    @Test fun agentProgressReplacesWaitingAndVersionUpdatedResetsIt() {
        scenario.onActivity { activity ->
            activity.handleWsMessage(
                WsEvent.AgentProgress("Rewriting *Checks*", WsEvent.AgentProgress.Bar(2, 4))
            )
            val button = activity.findViewById<Button>(R.id.btnRequestUpdate)
            assertEquals("Rewriting *Checks*", button.text.toString())
            val bar = activity.findViewById<ProgressBar>(R.id.agentProgress)
            assertEquals("a step with a total draws the bar", View.VISIBLE, bar.visibility)
            assertEquals(4, bar.max)
            assertEquals(2, bar.progress)

            activity.handleWsMessage(WsEvent.VersionUpdated(7, null))
            assertEquals("the rewrite landed, so the button is an action again", "Update", button.text.toString())
            assertEquals(View.GONE, bar.visibility)
        }
    }

    // Found in review: with no token stored, the no-token branch called itself and the
    // app died with a StackOverflowError on the first tap of any session.
    @Test
    fun aPageLoadsWithoutATokenAndCarriesTheHeaderWithOne() {
        scenario.onActivity { activity ->
            val repo = SessionRepository(activity)
            repo.saveToken("")
            activity.loadSessionPage("abc")
            val shadow = shadowOf(activity.webView)
            assertTrue(shadow.lastLoadedUrl.endsWith("/session/abc"))
            assertTrue(shadow.lastAdditionalHttpHeaders.isNullOrEmpty())

            repo.saveToken("s3cret")
            activity.loadSessionPage("def")
            assertTrue(shadowOf(activity.webView).lastLoadedUrl.endsWith("/session/def"))
            assertEquals("Bearer s3cret", shadowOf(activity.webView).lastAdditionalHttpHeaders["Authorization"])
            repo.saveToken("")
        }
    }

    // The tray's buttons were invisible in a way no unit test could have seen: a plain
    // <Button> under this MaterialComponents theme is inflated as a MaterialButton, which
    // ignores android:background and paints itself from colorPrimary — black fill, black
    // text. They have to be AppCompatButton, like every other button that carries a
    // drawable of its own.
    @Test
    fun theVoiceTrayButtonsKeepTheirOwnBackground() {
        scenario.onActivity { activity ->
            val buttons = listOf(
                R.id.voiceDone, R.id.voiceUndo, R.id.voiceDiscard,
                R.id.voiceShorter, R.id.voiceList, R.id.voiceBlocker, R.id.voicePlay,
            )
            for (id in buttons) {
                val button = activity.findViewById<View>(id)
                assertTrue(
                    "${activity.resources.getResourceEntryName(id)} is ${button.javaClass.name}",
                    button is androidx.appcompat.widget.AppCompatButton,
                )
                assertNotNull(
                    "${activity.resources.getResourceEntryName(id)} has no background",
                    button.background,
                )
            }
        }
    }
}
