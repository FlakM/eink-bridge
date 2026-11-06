package com.flakm.einkbridge

import org.json.JSONException
import org.json.JSONObject

/** A server message on the session socket, read without touching the UI. */
sealed class WsEvent {
    /**
     * The agent's rewrite landed. [consumed] carries only `consumed_strokes` and
     * `consumed_notes`, and is null when the rewrite consumed nothing.
     */
    data class VersionUpdated(val version: Int, val consumed: JSONObject?) : WsEvent()

    data class AnnotationResult(val version: Int) : WsEvent()

    /** [message] is blank when the agent only moved the bar; [bar] is null without a total. */
    data class AgentProgress(val message: String, val bar: Bar?) : WsEvent() {
        data class Bar(val step: Int, val of: Int)
    }

    data class ScreenshotRequested(val requestId: String) : WsEvent()

    data class Error(val message: String) : WsEvent()

    data object SessionSubmitted : WsEvent()

    data class Unknown(val type: String) : WsEvent()

    companion object {
        /** Null for a frame that is not JSON or asks for nothing. */
        fun parse(text: String): WsEvent? =
            try {
                from(JSONObject(text))
            } catch (_: JSONException) {
                null
            }

        fun from(json: JSONObject): WsEvent? = when (val type = json.optString("type", "")) {
            "version_updated" -> VersionUpdated(json.optInt("version"), consumed(json))
            "annotation_result" -> AnnotationResult(json.optInt("version"))
            "agent_progress" -> {
                val step = json.optInt("step")
                val of = json.optInt("of")
                AgentProgress(
                    json.optString("message"),
                    AgentProgress.Bar(step, of).takeIf { step > 0 && of > 0 },
                )
            }
            "screenshot_requested" ->
                json.optString("request_id").takeIf { it.isNotEmpty() }?.let(::ScreenshotRequested)
            "error" -> Error(json.optString("message", "Unknown error"))
            "session_submitted" -> SessionSubmitted
            else -> Unknown(type)
        }

        private fun consumed(json: JSONObject): JSONObject? {
            val out = JSONObject()
            for (key in listOf("consumed_strokes", "consumed_notes")) {
                json.optJSONArray(key)?.takeIf { it.length() > 0 }?.let { out.put(key, it) }
            }
            return out.takeIf { it.length() > 0 }
        }
    }
}
