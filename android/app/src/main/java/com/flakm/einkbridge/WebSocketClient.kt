package com.flakm.einkbridge

import android.util.Log
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit

private const val TAG = "EinkWS"

class WebSocketClient(
    private val serverUrl: String,
    private val sessionId: String,
    private val onMessage: (type: String, json: JSONObject) -> Unit,
    private val onClosed: () -> Unit,
) {
    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    @Volatile private var stopped = false
    private val retry = Runnable { if (!stopped) connect() }
    private val client = OkHttpClient.Builder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(10, TimeUnit.SECONDS)
        .build()

    private var ws: WebSocket? = null

    fun connect() {
        stopped = false
        handler.removeCallbacks(retry)
        val wsUrl = serverUrl
            .replaceFirst("http://", "ws://")
            .replaceFirst("https://", "wss://") + "/ws/$sessionId"
        Log.i(TAG, "connecting to $wsUrl")
        val request = Request.Builder().url(wsUrl).build()
        ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                Log.i(TAG, "connected session=$sessionId")
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                try {
                    val json = JSONObject(text)
                    val type = json.optString("type", "")
                    Log.d(TAG, "message type=$type session=$sessionId")
                    onMessage(type, json)
                } catch (e: Exception) {
                    Log.w(TAG, "failed to parse message session=$sessionId", e)
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Log.i(TAG, "closed code=$code reason=$reason session=$sessionId")
                onClosed()
                if (!stopped) handler.postDelayed(retry, 2000)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Log.w(TAG, "failed session=$sessionId code=${response?.code}", t)
                onClosed()
                if (!stopped) handler.postDelayed(retry, 2000)
            }
        })
    }

    fun disconnect() {
        stopped = true
        handler.removeCallbacks(retry)
        Log.i(TAG, "disconnect session=$sessionId")
        ws?.close(1000, null)
        ws = null
    }

    fun send(json: JSONObject) {
        val type = json.optString("type", "unknown")
        if (ws != null) {
            Log.i(TAG, "send type=$type session=$sessionId")
            ws?.send(json.toString())
        } else {
            Log.e(TAG, "send type=$type session=$sessionId — ws is null, message dropped!")
        }
    }
}
