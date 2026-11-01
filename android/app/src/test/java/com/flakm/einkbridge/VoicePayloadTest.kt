package com.flakm.einkbridge

import android.graphics.Color
import org.json.JSONArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A spoken note travels as an annotation group with no strokes: the anchor from the tap,
 * the cleaned words in `recognized_text`, and the server's own note under `voice`,
 * verbatim. Losing any of the three turns it into a pen note the agent cannot tell apart.
 */
class VoicePayloadTest {
    private val note = """{"raw":"so um drop it","edits":[],"repairs":[{"heard":"re-anchor rs","wrote":"reanchor.rs"}],"confidence":0.81,"stt":{"provider":"whisper","model":"small.en","language":"en"},"seconds":6.2,"audio":"/api/sessions/abc/voice/0f.wav","severity":"note"}"""
    private val ref = ElementRef("s-intro", "P", "The cache column")
    private val voice = BindGroup(
        id = 3, color = Color.RED, strokeIndices = emptySet(), elementIndices = listOf(4), elementRefs = listOf(ref),
        markerDocX = 10f, markerDocY = 20f, recognizedText = "drop it", source = "voice", voiceJson = note, severity = "note",
    )

    @Test fun `a voice group with no strokes is still submitted, with its source and note`() {
        val (groups, unanchored) = bindGroupsToAnnotations(emptyList(), listOf(voice))
        assertEquals(1, groups.size)
        assertTrue(unanchored.isEmpty())
        val json = JSONArray(annotationsToJson(groups, unanchored)).getJSONObject(0)
        assertEquals("explicit", json.getJSONObject("anchor").getString("type"))
        assertEquals("The cache column", json.getJSONObject("anchor").getJSONArray("elements").getJSONObject(0).getString("text"))
        assertEquals("drop it", json.getString("recognized_text"))
        assertEquals("voice", json.getString("source"))
        assertEquals(0, json.getJSONArray("strokes").length())
        val sent = json.getJSONObject("voice")
        assertEquals("so um drop it", sent.getString("raw"))
        assertEquals("reanchor.rs", sent.getJSONArray("repairs").getJSONObject(0).getString("wrote"))
        assertEquals("whisper", sent.getJSONObject("stt").getString("provider"))
        assertEquals(6.2, sent.getDouble("seconds"), 1e-9)
    }

    @Test fun `a pen group says nothing about voice`() {
        val pen = voice.copy(source = null, voiceJson = null, strokeIndices = setOf(0))
        val stroke = Stroke(listOf(0f to 0f, 5f to 5f), 3f)
        val (groups, _) = bindGroupsToAnnotations(listOf(stroke), listOf(pen))
        val json = JSONArray(annotationsToJson(groups, emptyList())).getJSONObject(0)
        assertFalse(json.has("source"))
        assertFalse(json.has("voice"))
    }

    @Test fun `the voice fields survive the tablet's own persistence`() {
        val back = bindGroupsFromJson(bindGroupsToJson(listOf(voice))).single()
        assertEquals("voice", back.source)
        assertEquals(note, back.voiceJson)
        assertEquals("note", back.severity)
        assertTrue(back.isVoice)
        val pen = bindGroupsFromJson(bindGroupsToJson(listOf(voice.copy(source = null, voiceJson = null, severity = null)))).single()
        assertNull(pen.source)
        assertFalse(pen.isVoice)
    }

    @Test fun `the card the page draws comes from the group`() {
        val card = voiceCard(2, voice)
        assertEquals(2, card.n)
        assertEquals(4, card.elementIndex)
        assertEquals("drop it", card.text)
        assertEquals("so um drop it", card.raw)
        assertEquals("spoken · 6s", card.state)
        assertFalse(card.guarded)
        val guarded = voiceCard(1, voice.copy(voiceJson = """{"raw":"x","seconds":3,"guarded":true}"""))
        assertEquals("heard, not cleaned · 3s", guarded.state)
        assertTrue(guarded.guarded)
        assertEquals("blocker · 6s", voiceCard(1, voice.copy(severity = "blocker")).state)
    }
}
