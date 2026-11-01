package com.flakm.einkbridge

import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The tray's reducer and the pure pieces around it. No Android here: every transition
 * the tablet goes through while a note is spoken is a row below.
 */
class VoiceCaptureTest {
    private fun frame(json: String) = parseVoiceFrame(json)!!
    private val ready = frame("""{"type":"ready","seq":0,"capture":"abc"}""")
    private val final = frame("""{"type":"final","seq":7,"raw":"so um drop it","text":"drop it","seconds":6.2,"confidence":0.8,"guarded":false}""")

    @Test fun `ready starts listening with an empty transcript`() {
        val state = voiceTrayState(VoiceTrayState(text = "stale"), ready)
        assertEquals(VoicePhase.LISTENING, state.phase)
        assertEquals("", state.text)
        assertEquals("● listening 0:07", state.statusLine("0:07"))
    }

    @Test fun `a partial updates the words while listening`() {
        val state = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), frame("""{"type":"partial","seq":3,"text":"so um drop"}"""))
        assertEquals("so um drop", state.text)
    }

    @Test fun `a partial older than the last final is dropped`() {
        val afterFinal = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), final)
        val late = voiceTrayState(afterFinal, frame("""{"type":"partial","seq":5,"text":"so um dro"}"""))
        assertSame("a late partial cannot overwrite the final", afterFinal, late)
        assertEquals("drop it", late.text)
    }

    @Test fun `the final shows the cleaned words and keeps what was heard`() {
        val state = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), final)
        assertEquals(VoicePhase.EDITING, state.phase)
        assertEquals("drop it", state.text)
        assertEquals("so um drop it", state.raw)
        assertEquals("spoken · 0:06", state.statusLine("0:06"))
    }

    @Test fun `a guarded final says the words were not cleaned`() {
        val guarded = frame("""{"type":"final","seq":7,"raw":"so um drop it","text":"so um drop it","seconds":6,"guarded":true}""")
        val state = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), guarded)
        assertTrue(state.guarded)
        assertEquals("heard, not cleaned · 0:06", state.statusLine("0:06"))
    }

    @Test fun `an applied edit replaces the words and a refused one explains itself`() {
        val editing = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), final)
        val applied = voiceTrayState(editing, frame("""{"type":"edit","seq":8,"instruction":"mark it a blocker","applied":true,"text":"drop it","severity":"blocker","edits":1}"""))
        assertEquals("blocker", applied.severity)
        assertNull(applied.message)
        assertEquals("blocker · 0:06", applied.statusLine("0:06"))
        val refused = voiceTrayState(applied, frame("""{"type":"edit","seq":9,"instruction":"make it rhyme","applied":false,"text":"drop it","severity":"blocker","edits":1,"message":"the cleaner invented a word"}"""))
        assertEquals("drop it", refused.text)
        assertEquals("blocker · 0:06 — the cleaner invented a word", refused.statusLine("0:06"))
    }

    @Test fun `an edit before the final is ignored`() {
        val listening = voiceTrayState(VoiceTrayState(), ready)
        val state = voiceTrayState(listening, frame("""{"type":"edit","seq":2,"instruction":"shorter","applied":true,"text":"x","severity":"note","edits":1}"""))
        assertSame(listening, state)
    }

    @Test fun `an error ends the capture with its reason`() {
        val state = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), frame("""{"type":"error","seq":4,"message":"the recogniser failed"}"""))
        assertEquals(VoicePhase.FAILED, state.phase)
        assertEquals("the recogniser failed", state.statusLine("0:03"))
        assertFalse(state.listening)
    }

    @Test fun `done carries the note verbatim`() {
        val editing = voiceTrayState(voiceTrayState(VoiceTrayState(), ready), final)
        val state = voiceTrayState(editing, frame("""{"type":"done","seq":10,"text":"drop it","severity":"note","note":{"raw":"so um drop it","edits":[],"seconds":6.2}}"""))
        assertEquals(VoicePhase.DONE, state.phase)
        assertEquals("""{"raw":"so um drop it","edits":[],"seconds":6.2}""", state.note)
    }

    @Test fun `a refused upgrade is said in words`() {
        assertEquals("no speech recogniser on the server", voiceRefused(VoiceTrayState(), 503, null).message)
        assertEquals("this session is gone", voiceRefused(VoiceTrayState(), 404, null).message)
        assertEquals("a capture is already open", voiceRefused(VoiceTrayState(), 409, "a capture is already open").message)
        assertEquals("could not reach the server", voiceRefused(VoiceTrayState(), null, null).message)
        assertEquals(VoicePhase.FAILED, voiceRefused(VoiceTrayState(), 500, null).phase)
    }

    @Test fun `an unknown frame is ignored rather than crashing the tray`() {
        assertNull(parseVoiceFrame("""{"type":"shrug","seq":1}"""))
        assertNull(parseVoiceFrame("not json"))
    }

    @Test fun `the socket names the session and carries the token in the query`() {
        assertEquals("ws://10.0.0.5:3333/api/sessions/abc/voice", voiceSocketUrl("http://10.0.0.5:3333/", "abc", null))
        assertEquals("wss://host/api/sessions/abc/voice?token=s%26t", voiceSocketUrl("https://host", "abc", " s&t "))
    }

    @Test fun `elapsed is fixed width minutes and seconds`() {
        assertEquals("0:07", formatElapsed(7_400))
        assertEquals("1:05", formatElapsed(65_000))
        assertEquals("0:00", formatElapsed(-5))
    }

    @Test fun `a strike is wide and flat, a letter is not`() {
        assertTrue(isStrike(listOf(10f to 100f, 60f to 102f, 120f to 99f)))
        assertFalse("a short dash", isStrike(listOf(10f to 100f, 40f to 100f)))
        assertFalse("a tall stroke", isStrike(listOf(10f to 100f, 100f to 100f, 100f to 160f)))
        assertFalse(isStrike(listOf(10f to 10f)))
    }

    @Test fun `sentences split the way the page splits them`() {
        assertEquals(listOf("Drop the cache column.", "The index is enough!", "Really?"), splitSentences("Drop the cache column. The index is enough!  Really?"))
        assertEquals(listOf("- one", "- two"), splitSentences("- one\n- two"))
        assertEquals("The index is enough.", dropSentence("Drop the cache column. The index is enough.", 0))
        assertEquals("- one\n- three", dropSentence("- one\n- two\n- three", 1))
        assertEquals("unchanged", dropSentence("unchanged", 4))
    }

    @Test fun `a strike is recorded as an edit on the stored note`() {
        val note = noteWithEdit("""{"raw":"a. b.","edits":[{"instruction":"shorter","before":"x","after":"y"}]}""", "strike 2", "a. b.", "a.")
        val edits = org.json.JSONObject(note).getJSONArray("edits")
        assertEquals(2, edits.length())
        assertEquals("strike 2", edits.getJSONObject(1).getString("instruction"))
        assertEquals("a.", edits.getJSONObject(1).getString("after"))
        assertEquals("a. b.", org.json.JSONObject(note).getString("raw"))
    }

    private class Recording(private val code: Int) {
        var request: Request? = null
        val client: OkHttpClient = OkHttpClient.Builder()
            .addInterceptor { chain ->
                request = chain.request()
                Response.Builder().request(chain.request()).protocol(Protocol.HTTP_1_1).code(code).message("x")
                    .body("RIFFwav".toByteArray().toResponseBody(null)).build()
            }
            .build()

        fun body(): String = Buffer().also { request!!.body!!.writeTo(it) }.readUtf8()
    }

    @Test fun `play asks the server to read the note and gets the wav back`() {
        val recording = Recording(200)
        val result = fetchSpeech(recording.client, "http://10.0.0.5:3333/", "abc", "drop it")
        assertTrue(result is SpeechResult.Audio)
        assertEquals("RIFFwav", String((result as SpeechResult.Audio).wav))
        val request = recording.request!!
        assertEquals("POST", request.method)
        assertEquals("http://10.0.0.5:3333/api/sessions/abc/speech", request.url.toString())
        assertTrue(request.body?.contentType().toString().startsWith("application/json"))
        assertEquals("""{"text":"drop it"}""", recording.body())
    }

    @Test fun `a server with no voice is reported, not played`() {
        val result = fetchSpeech(Recording(503).client, "http://h", "abc", "x")
        assertEquals(503, (result as SpeechResult.Refused).code)
    }
}
