package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class WsEventTest {

    @Test fun aRewriteCarriesItsVersionAndWhatItConsumed() {
        val event = WsEvent.parse(
            """{"type":"version_updated","version":3,"feedback_id":"f1",
               "consumed_strokes":[[[10,20],[30,40]]],
               "consumed_notes":[{"text":"needs an example","anchor":["Second."]}]}"""
        ) as WsEvent.VersionUpdated
        assertEquals(3, event.version)
        val consumed = event.consumed!!
        assertEquals(setOf("consumed_strokes", "consumed_notes"), consumed.keys().asSequence().toSet())
        assertEquals(10, consumed.getJSONArray("consumed_strokes").getJSONArray(0).getJSONArray(0).getInt(0))
        assertEquals("needs an example", consumed.getJSONArray("consumed_notes").getJSONObject(0).getString("text"))
    }

    @Test fun aRewriteThatConsumedOnlySpokenNotesStillNamesThem() {
        val event = WsEvent.parse(
            """{"type":"version_updated","version":2,"consumed_notes":[{"text":"x","anchor":[]}]}"""
        ) as WsEvent.VersionUpdated
        assertEquals(setOf("consumed_notes"), event.consumed!!.keys().asSequence().toSet())
    }

    @Test fun aRewriteThatConsumedNothingHasNothingToDrop() {
        assertEquals(
            WsEvent.VersionUpdated(4, null),
            WsEvent.parse("""{"type":"version_updated","version":4,"consumed_strokes":[]}"""),
        )
    }

    @Test fun anAnnotationResultCarriesItsVersion() {
        assertEquals(
            WsEvent.AnnotationResult(5),
            WsEvent.parse("""{"type":"annotation_result","version":5,"feedback_id":"f"}"""),
        )
    }

    @Test fun progressWithATotalDrawsTheBarAndWithoutOneDoesNot() {
        assertEquals(
            WsEvent.AgentProgress("Writing", WsEvent.AgentProgress.Bar(2, 4)),
            WsEvent.parse("""{"type":"agent_progress","message":"Writing","step":2,"of":4}"""),
        )
        assertEquals(
            WsEvent.AgentProgress("Writing", null),
            WsEvent.parse("""{"type":"agent_progress","message":"Writing","step":2}"""),
        )
        assertEquals(
            WsEvent.AgentProgress("", null),
            WsEvent.parse("""{"type":"agent_progress","step":0,"of":3}"""),
        )
    }

    @Test fun aScreenshotRequestNeedsAnId() {
        assertEquals(
            WsEvent.ScreenshotRequested("abc"),
            WsEvent.parse("""{"type":"screenshot_requested","request_id":"abc"}"""),
        )
        assertNull(WsEvent.parse("""{"type":"screenshot_requested"}"""))
    }

    @Test fun anErrorWithoutAMessageStillSaysSomething() {
        assertEquals(WsEvent.Error("boom"), WsEvent.parse("""{"type":"error","message":"boom"}"""))
        assertEquals(WsEvent.Error("Unknown error"), WsEvent.parse("""{"type":"error"}"""))
    }

    @Test fun submittedIsItsOwnEvent() {
        assertEquals(WsEvent.SessionSubmitted, WsEvent.parse("""{"type":"session_submitted"}"""))
    }

    @Test fun anUnknownTypeIsNamedAndAFrameThatIsNotJsonIsDropped() {
        assertEquals(WsEvent.Unknown("pong"), WsEvent.parse("""{"type":"pong"}"""))
        assertEquals(WsEvent.Unknown(""), WsEvent.parse("""{"version":1}"""))
        assertNull(WsEvent.parse("not json"))
        assertNull(WsEvent.parse("[1,2]"))
    }
}
