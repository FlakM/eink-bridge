package com.flakm.einkbridge

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit

class WebSocketClient(
    private val serverUrl: String,
    private val sessionId: String,
    private val onMessage: (type: String, json: JSONObject) -> Unit,
    private val onClosed: () -> Unit,
) {
    private val client = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build()

    private var ws: WebSocket? = null

    fun connect() {
        val wsUrl = serverUrl
            .replaceFirst("http://", "ws://")
            .replaceFirst("https://", "wss://") + "/ws/$sessionId"
        val request = Request.Builder().url(wsUrl).build()
        ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, text: String) {
                try {
                    val json = JSONObject(text)
                    val type = json.optString("type", "")
                    onMessage(type, json)
                } catch (_: Exception) {}
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                onClosed()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                onClosed()
            }
        })
    }

    fun disconnect() {
        ws?.close(1000, null)
        ws = null
    }

    fun send(json: JSONObject) {
        ws?.send(json.toString())
    }
}
