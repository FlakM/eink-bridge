package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.net.URLEncoder
import java.util.concurrent.TimeUnit
import kotlin.math.abs

private const val TAG = "EinkVoice"

internal const val VOICE_SAMPLE_RATE = 16_000
/** 20 ms of 16-bit mono at 16 kHz. */
internal const val VOICE_CHUNK_BYTES = 640

/** One frame from the server on the voice socket. Everything carries `seq`. */
internal sealed class VoiceFrame(val seq: Int) {
    class Ready(seq: Int, val capture: String) : VoiceFrame(seq)
    class Partial(seq: Int, val text: String) : VoiceFrame(seq)
    class Final(seq: Int, val raw: String, val text: String, val seconds: Float, val guarded: Boolean) : VoiceFrame(seq)
    class Edit(seq: Int, val instruction: String, val applied: Boolean, val text: String, val severity: String, val edits: Int, val message: String?) : VoiceFrame(seq)
    class Done(seq: Int, val text: String, val severity: String, val note: String) : VoiceFrame(seq)
    class Error(seq: Int, val message: String) : VoiceFrame(seq)
}

internal fun parseVoiceFrame(text: String): VoiceFrame? {
    val json = runCatching { JSONObject(text) }.getOrNull() ?: return null
    val seq = json.optInt("seq", 0)
    return when (json.optString("type")) {
        "ready" -> VoiceFrame.Ready(seq, json.optString("capture"))
        "partial" -> VoiceFrame.Partial(seq, json.optString("text"))
        "final" -> VoiceFrame.Final(seq, json.optString("raw"), json.optString("text"),
            json.optDouble("seconds", 0.0).toFloat(), json.optBoolean("guarded", false))
        "edit" -> VoiceFrame.Edit(seq, json.optString("instruction"), json.optBoolean("applied", false),
            json.optString("text"), json.optString("severity", "note"), json.optInt("edits", 0),
            json.optString("message").takeIf { it.isNotEmpty() })
        "done" -> VoiceFrame.Done(seq, json.optString("text"), json.optString("severity", "note"),
            extractObject(text, "note") ?: "{}")
        "error" -> VoiceFrame.Error(seq, json.optString("message", "the server could not take the note"))
        else -> null
    }
}

/**
 * The text of the object under [key], exactly as it was sent. Re-serialising through
 * JSONObject reorders keys, and the note is forwarded to the server verbatim.
 */
internal fun extractObject(json: String, key: String): String? {
    val at = json.indexOf("\"$key\"")
    if (at < 0) return null
    val start = json.indexOf('{', json.indexOf(':', at))
    if (start < 0) return null
    var depth = 0
    var inString = false
    var i = start
    while (i < json.length) {
        val c = json[i]
        when {
            inString && c == '\\' -> i++
            inString && c == '"' -> inString = false
            !inString && c == '"' -> inString = true
            !inString && c == '{' -> depth++
            !inString && c == '}' -> { depth--; if (depth == 0) return json.substring(start, i + 1) }
        }
        i++
    }
    return null
}

internal enum class VoicePhase { AIMING, LISTENING, EDITING, DONE, FAILED }

/**
 * What the tray shows. A pure value: the reducer below is the only thing that changes it,
 * so every transition the tablet can go through is a test with no Android in it.
 */
internal data class VoiceTrayState(
    val phase: VoicePhase = VoicePhase.AIMING,
    val text: String = "",
    val raw: String? = null,
    val guarded: Boolean = false,
    val severity: String = "note",
    val lastFinalSeq: Int = -1,
    val message: String? = null,
    val note: String? = null,
) {
    val listening get() = phase == VoicePhase.LISTENING
    val editing get() = phase == VoicePhase.EDITING

    /** The first row of the tray. `elapsed` is the tablet's own clock, `m:ss`. */
    fun statusLine(elapsed: String): String {
        val head = when (phase) {
            VoicePhase.AIMING -> "Tap the block you want to talk about"
            VoicePhase.LISTENING -> "● listening $elapsed"
            VoicePhase.EDITING, VoicePhase.DONE -> when {
                guarded -> "heard, not cleaned · $elapsed"
                severity == "blocker" -> "blocker · $elapsed"
                else -> "spoken · $elapsed"
            }
            VoicePhase.FAILED -> message ?: "could not take the note"
        }
        return if (phase != VoicePhase.FAILED && message != null) "$head — $message" else head
    }
}

/** A late partial cannot overwrite a final, and nothing after `done` or an error changes. */
internal fun voiceTrayState(previous: VoiceTrayState, frame: VoiceFrame): VoiceTrayState = when (frame) {
    is VoiceFrame.Ready -> previous.copy(phase = VoicePhase.LISTENING, text = "", message = null)
    is VoiceFrame.Partial ->
        if (previous.phase != VoicePhase.LISTENING || frame.seq < previous.lastFinalSeq) previous
        else previous.copy(text = frame.text)
    is VoiceFrame.Final ->
        if (previous.phase == VoicePhase.DONE || previous.phase == VoicePhase.FAILED) previous
        else previous.copy(phase = VoicePhase.EDITING, text = frame.text, raw = frame.raw,
            guarded = frame.guarded, lastFinalSeq = frame.seq, message = null)
    is VoiceFrame.Edit ->
        if (!previous.editing) previous
        else previous.copy(text = frame.text, severity = frame.severity,
            message = if (frame.applied) null else frame.message ?: "not applied")
    is VoiceFrame.Done -> previous.copy(phase = VoicePhase.DONE, text = frame.text, severity = frame.severity, note = frame.note, message = null)
    is VoiceFrame.Error -> previous.copy(phase = VoicePhase.FAILED, message = frame.message)
}

/** The socket was refused before any frame: the HTTP code is the whole story. */
internal fun voiceRefused(previous: VoiceTrayState, code: Int?, reason: String?): VoiceTrayState =
    previous.copy(phase = VoicePhase.FAILED, message = when (code) {
        404 -> "this session is gone"
        409 -> reason?.takeIf { it.isNotBlank() } ?: "the session is not open for notes"
        503 -> "no speech recogniser on the server"
        null -> reason?.takeIf { it.isNotBlank() } ?: "could not reach the server"
        else -> "the server refused ($code)"
    })

internal fun formatElapsed(ms: Long): String {
    val s = (ms / 1000).coerceAtLeast(0)
    return "%d:%02d".format(s / 60, s % 60)
}

internal fun voiceSocketUrl(serverUrl: String, sessionId: String, token: String?): String {
    val base = serverUrl.trimEnd('/')
        .replaceFirst("http://", "ws://")
        .replaceFirst("https://", "wss://")
    val value = token?.trim().orEmpty()
    val tokenParam = if (value.isEmpty()) "" else "?token=" + URLEncoder.encode(value, "UTF-8")
    return "$base/api/sessions/$sessionId/voice$tokenParam"
}

internal fun speechUrl(serverUrl: String, sessionId: String): String =
    "${serverUrl.trimEnd('/')}/api/sessions/$sessionId/speech"

internal sealed class SpeechResult {
    class Audio(val wav: ByteArray) : SpeechResult()
    class Refused(val code: Int) : SpeechResult()
}

/** Phase E: ask the server to read a note aloud. Blocking; call off the main thread. */
internal fun fetchSpeech(client: OkHttpClient, serverUrl: String, sessionId: String, text: String): SpeechResult =
    client.newCall(
        Request.Builder()
            .url(speechUrl(serverUrl, sessionId))
            .post(JSONObject().put("text", text).toString().toRequestBody("application/json".toMediaType()))
            .build()
    ).execute().use { response ->
        if (response.isSuccessful) SpeechResult.Audio(response.body?.bytes() ?: ByteArray(0))
        else SpeechResult.Refused(response.code)
    }

/**
 * A stroke that is a line across a sentence, not a letter: wide, flat, and long enough to
 * mean it. Document units.
 */
internal fun isStrike(points: List<Pair<Float, Float>>): Boolean {
    if (points.size < 2) return false
    val width = points.maxOf { it.first } - points.minOf { it.first }
    val height = points.maxOf { it.second } - points.minOf { it.second }
    return width > STRIKE_MIN_WIDTH_DOC && width > 3f * height
}

internal const val STRIKE_MIN_WIDTH_DOC = 60f

/** The page splits a card into sentences the same way: lines first, then `.!?` + space. */
internal fun splitSentences(text: String): List<String> =
    text.split('\n').flatMap { line -> line.split(Regex("(?<=[.!?])\\s+")) }
        .map { it.trim() }.filter { it.isNotEmpty() }

internal fun dropSentence(text: String, index: Int): String {
    val sentences = splitSentences(text)
    if (index !in sentences.indices) return text
    val list = text.lines().count { it.trimStart().startsWith("- ") } > 1
    val kept = sentences.filterIndexed { i, _ -> i != index }
    return if (list) kept.joinToString("\n") else kept.joinToString(" ")
}

/** Append one applied instruction to a stored note's `edits`, keeping everything else verbatim. */
internal fun noteWithEdit(noteJson: String, instruction: String, before: String, after: String): String {
    val note = runCatching { JSONObject(noteJson) }.getOrElse { JSONObject() }
    val edits = note.optJSONArray("edits") ?: org.json.JSONArray().also { note.put("edits", it) }
    edits.put(JSONObject().put("instruction", instruction).put("before", before).put("after", after))
    return note.toString()
}

/** Where the microphone's bytes come from; swapped for a fake in tests. */
internal fun interface AudioSource {
    /** Blocks, calling [onChunk] until [stop] is set. */
    fun run(stop: () -> Boolean, onChunk: (ByteArray) -> Unit)
}

@SuppressLint("MissingPermission")
internal class MicrophoneSource : AudioSource {
    override fun run(stop: () -> Boolean, onChunk: (ByteArray) -> Unit) {
        val min = AudioRecord.getMinBufferSize(VOICE_SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val record = AudioRecord(MediaRecorder.AudioSource.MIC, VOICE_SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(min, VOICE_CHUNK_BYTES * 8))
        if (record.state != AudioRecord.STATE_INITIALIZED) {
            Log.w(TAG, "microphone did not initialise")
            return
        }
        try {
            record.startRecording()
            val chunk = ByteArray(VOICE_CHUNK_BYTES)
            while (!stop()) {
                var filled = 0
                while (filled < chunk.size && !stop()) {
                    val n = record.read(chunk, filled, chunk.size - filled)
                    if (n <= 0) return
                    filled += n
                }
                if (filled == chunk.size) onChunk(chunk.copyOf())
            }
        } finally {
            runCatching { record.stop() }
            record.release()
        }
    }
}

/**
 * One capture: the socket to the server and the thread that feeds it the microphone.
 * Frames come back on [onFrame] on the main thread; a refused upgrade on [onRefused].
 */
internal class VoiceSession(
    serverUrl: String,
    sessionId: String,
    token: String?,
    private val source: AudioSource = MicrophoneSource(),
    private val onFrame: (VoiceFrame) -> Unit,
    private val onRefused: (code: Int?, reason: String?) -> Unit,
    private val onClosed: () -> Unit,
) {
    private val url = voiceSocketUrl(serverUrl, sessionId, token)
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    private val client = OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS).pingInterval(10, TimeUnit.SECONDS).build()
    private var ws: WebSocket? = null
    @Volatile private var micStopped = false
    @Volatile private var closed = false
    private var mic: Thread? = null

    fun open() {
        Log.i(TAG, "voice socket connecting $url")
        ws = client.newWebSocket(Request.Builder().url(url).build(), object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                startMicrophone(webSocket)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val frame = parseVoiceFrame(text) ?: return
                if (frame is VoiceFrame.Final || frame is VoiceFrame.Done || frame is VoiceFrame.Error) micStopped = true
                main.post { if (!closed) onFrame(frame) }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                micStopped = true
                main.post { if (!closed) { closed = true; onClosed() } }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                micStopped = true
                Log.w(TAG, "voice socket failed code=${response?.code}", t)
                val code = response?.code
                val reason = response?.body?.let { runCatching { it.string() }.getOrNull() }?.takeIf { it.length < 200 }
                main.post {
                    if (closed) return@post
                    closed = true
                    onRefused(code, reason ?: t.message)
                }
            }
        })
    }

    private fun startMicrophone(socket: WebSocket) {
        mic = Thread({
            source.run({ micStopped || closed }) { chunk -> socket.send(chunk.toByteString()) }
        }, "eink-voice-mic").also { it.start() }
    }

    fun send(type: String, instruction: String? = null) {
        val json = JSONObject().put("type", type)
        instruction?.let { json.put("instruction", it) }
        if (type == "stop" || type == "done" || type == "discard") micStopped = true
        ws?.send(json.toString())
    }

    fun close() {
        closed = true
        micStopped = true
        ws?.close(1000, null)
        ws = null
    }
}

/** A tap, not a drag: the tap that picks the block a note is spoken at. */
internal fun isTap(downX: Float, downY: Float, upX: Float, upY: Float): Boolean =
    abs(upX - downX) < 30f && abs(upY - downY) < 30f
