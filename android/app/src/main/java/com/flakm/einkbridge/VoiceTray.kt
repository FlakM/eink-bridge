package com.flakm.einkbridge

import android.media.MediaPlayer
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.TextView
import com.onyx.android.sdk.api.device.epd.EpdController
import com.onyx.android.sdk.api.device.epd.UpdateMode
import okhttp3.OkHttpClient
import org.json.JSONObject
import java.io.File

private const val TAG = "EinkVoiceTray"

/**
 * The strip above the toolbar while a note is spoken.
 *
 * Owns the capture: the socket, the microphone thread, the elapsed clock. The page and
 * the overlay only hear the outcome: a note accepted onto its group, or the group thrown
 * away.
 */
internal class VoiceTrayController(
    private val tray: View,
    private val client: OkHttpClient,
    private val serverUrl: () -> String,
    private val token: () -> String?,
    private val onAccepted: (groupId: Int, text: String, severity: String, note: String) -> Unit,
    private val onDiscarded: (groupId: Int) -> Unit,
    private val onFinished: () -> Unit,
    private val onStatus: (String) -> Unit,
    private val openSession: (sessionId: String, onFrame: (VoiceFrame) -> Unit, onRefused: (Int?, String?) -> Unit, onClosed: () -> Unit) -> VoiceSession =
        { sid, onFrame, onRefused, onClosed -> VoiceSession(serverUrl(), sid, token(), onFrame = onFrame, onRefused = onRefused, onClosed = onClosed) },
) {
    private val stateText = tray.findViewById<TextView>(R.id.voiceState)
    private val transcript = tray.findViewById<TextView>(R.id.voiceText)
    private val done = tray.findViewById<Button>(R.id.voiceDone)
    private val undo = tray.findViewById<Button>(R.id.voiceUndo)
    private val discard = tray.findViewById<Button>(R.id.voiceDiscard)
    private val edits = tray.findViewById<View>(R.id.voiceEdits)
    private val play = tray.findViewById<Button>(R.id.voicePlay)
    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            render()
            if (state.listening) handler.postDelayed(this, 1000)
        }
    }

    internal var state = VoiceTrayState()
        private set
    private var session: VoiceSession? = null
    private var sessionId: String? = null
    private var groupId: Int? = null
    private var startedAt = 0L
    private var stoppedAt: Long? = null

    val active get() = tray.visibility == View.VISIBLE

    init {
        transcript.movementMethod = android.text.method.ScrollingMovementMethod()
        done.setOnClickListener { onDone() }
        undo.setOnClickListener { if (state.editing) session?.send("undo") }
        discard.setOnClickListener { discardCapture() }
        tray.findViewById<Button>(R.id.voiceShorter).setOnClickListener { edit("shorter") }
        tray.findViewById<Button>(R.id.voiceList).setOnClickListener { edit("turn that into a list") }
        tray.findViewById<Button>(R.id.voiceBlocker).setOnClickListener { edit("mark it a blocker") }
        play.setOnClickListener { playCurrent() }
    }

    /** Speak was pressed: wait for the tap that names the block. */
    fun showAiming(sessionId: String) {
        this.sessionId = sessionId
        state = VoiceTrayState()
        groupId = null
        tray.visibility = View.VISIBLE
        render()
    }

    /** The block was tapped: open the socket and start listening. */
    fun startCapture(group: BindGroup) {
        val sid = sessionId ?: return
        session?.close()
        groupId = group.id
        startedAt = SystemClock.elapsedRealtime()
        stoppedAt = null
        state = VoiceTrayState(phase = VoicePhase.AIMING, message = "connecting")
        render()
        session = openSession(sid, ::onFrame, ::onRefused) { onClosed() }.also { it.open() }
    }

    /** The mode was left by something other than this tray: nothing survives. */
    fun leave() {
        if (session != null) discardCapture(finish = false)
        hide()
    }

    private fun onFrame(frame: VoiceFrame) {
        val previous = state
        state = voiceTrayState(previous, frame)
        if (frame is VoiceFrame.Ready) {
            startedAt = SystemClock.elapsedRealtime()
            fastRefresh(true)
            handler.removeCallbacks(tick)
            handler.post(tick)
        }
        if (previous.listening && !state.listening) {
            stoppedAt = SystemClock.elapsedRealtime()
            fastRefresh(false)
        }
        if (frame is VoiceFrame.Done) {
            val id = groupId
            val note = if (state.guarded) noteWithFlag(frame.note) else frame.note
            session?.close()
            session = null
            if (id != null && state.text.isNotBlank()) onAccepted(id, state.text, state.severity, note)
            else if (id != null) onDiscarded(id)
            hide()
            onFinished()
            return
        }
        if (frame is VoiceFrame.Error) {
            session?.close()
            session = null
            groupId?.let(onDiscarded)
            groupId = null
        }
        render()
    }

    private fun onRefused(code: Int?, reason: String?) {
        state = voiceRefused(state, code, reason)
        session = null
        groupId?.let(onDiscarded)
        groupId = null
        fastRefresh(false)
        render()
    }

    private fun onClosed() {
        if (state.phase == VoicePhase.DONE || state.phase == VoicePhase.FAILED) return
        session = null
        if (state.listening || state.editing) {
            state = state.copy(phase = VoicePhase.FAILED, message = "the server closed the capture")
            groupId?.let(onDiscarded)
            groupId = null
            render()
        }
    }

    private fun onDone() {
        when (state.phase) {
            VoicePhase.LISTENING -> session?.send("stop")
            VoicePhase.EDITING -> session?.send("done")
            VoicePhase.AIMING -> if (session == null) { hide(); onFinished() }
            VoicePhase.DONE, VoicePhase.FAILED -> { hide(); onFinished() }
        }
    }

    private fun edit(instruction: String) {
        if (state.editing) session?.send("edit", instruction)
    }

    private fun discardCapture(finish: Boolean = true) {
        session?.send("discard")
        session?.close()
        session = null
        groupId?.let(onDiscarded)
        groupId = null
        hide()
        if (finish) onFinished()
    }

    private fun hide() {
        handler.removeCallbacks(tick)
        fastRefresh(false)
        tray.visibility = View.GONE
        state = VoiceTrayState()
    }

    private fun elapsed(): String {
        val end = stoppedAt ?: SystemClock.elapsedRealtime()
        return formatElapsed(end - startedAt)
    }

    internal fun render() {
        stateText.text = state.statusLine(elapsed())
        if (transcript.text.toString() != state.text) {
            transcript.text = state.text
            transcript.post {
                val lines = transcript.layout?.lineCount ?: 0
                if (lines > transcript.maxLines) {
                    val y = transcript.layout.getLineTop(lines) - transcript.height
                    if (y > 0) transcript.scrollTo(0, y)
                }
            }
        }
        done.text = when (state.phase) {
            VoicePhase.DONE, VoicePhase.FAILED -> "Close"
            else -> "Done"
        }
        done.isEnabled = state.phase != VoicePhase.AIMING || session == null
        undo.isEnabled = state.editing
        discard.isEnabled = session != null
        edits.visibility = if (state.editing || state.phase == VoicePhase.DONE) View.VISIBLE else View.GONE
        play.isEnabled = state.text.isNotBlank()
    }

    private fun fastRefresh(on: Boolean) {
        runCatching {
            EpdController.setViewDefaultUpdateMode(tray, if (on) UpdateMode.DU else UpdateMode.GC)
            if (!on) EpdController.invalidate(tray, UpdateMode.GC)
        }
    }

    private fun playCurrent() {
        val sid = sessionId ?: return
        val text = state.text.takeIf { it.isNotBlank() } ?: return
        play.isEnabled = false
        Thread {
            val result = runCatching { fetchSpeech(client, serverUrl(), sid, text) }.getOrNull()
            handler.post {
                play.isEnabled = true
                when (result) {
                    is SpeechResult.Audio -> playWav(result.wav)
                    is SpeechResult.Refused -> onStatus(if (result.code == 503) "No voice on the server" else "Could not fetch the speech (${result.code})")
                    null -> onStatus("Could not reach the server")
                }
            }
        }.start()
    }

    private fun playWav(wav: ByteArray) {
        val file = File(tray.context.cacheDir, "eink-speech.wav")
        runCatching {
            file.writeBytes(wav)
            MediaPlayer().apply {
                setDataSource(file.path)
                setOnCompletionListener { it.release() }
                setOnErrorListener { mp, _, _ -> mp.release(); true }
                prepare()
                start()
            }
        }.onFailure { Log.w(TAG, "could not play the note", it); onStatus("Could not play the note") }
    }

    private fun noteWithFlag(note: String): String =
        runCatching { JSONObject(note).put("guarded", true).toString() }.getOrDefault(note)
}
