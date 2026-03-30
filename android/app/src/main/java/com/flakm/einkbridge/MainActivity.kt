package com.flakm.einkbridge

import android.annotation.SuppressLint
import android.content.SharedPreferences
import android.os.Bundle
import android.os.Vibrator
import android.view.View
import android.webkit.*
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.*
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONArray
import java.util.concurrent.TimeUnit

class MainActivity : AppCompatActivity() {
    private lateinit var webView: WebView
    private lateinit var sessionListContainer: View
    private lateinit var sessionList: RecyclerView
    private lateinit var serverInput: EditText
    private lateinit var serverUrlText: TextView
    private lateinit var adapter: SessionAdapter
    private lateinit var prefs: SharedPreferences

    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private var pollJob: Job? = null
    private var serverUrl = ""
    private var currentSessionId: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        prefs = getSharedPreferences("eink_bridge", MODE_PRIVATE)
        serverUrl = prefs.getString("server_url", "http://amd-pc:3333") ?: "http://amd-pc:3333"

        webView = findViewById(R.id.webView)
        sessionListContainer = findViewById(R.id.sessionListContainer)
        sessionList = findViewById(R.id.sessionList)
        serverInput = findViewById(R.id.serverInput)
        serverUrlText = findViewById(R.id.serverUrl)

        serverInput.setText(serverUrl)
        serverUrlText.text = if (serverUrl.isNotEmpty()) "Connected: $serverUrl" else "Not connected"

        adapter = SessionAdapter { session ->
            openSession(session.id)
        }
        sessionList.layoutManager = LinearLayoutManager(this)
        sessionList.adapter = adapter

        setupWebView()

        findViewById<Button>(R.id.connectBtn).setOnClickListener {
            val url = serverInput.text.toString().trimEnd('/')
            if (url.isNotEmpty()) {
                serverUrl = url
                prefs.edit().putString("server_url", url).apply()
                serverUrlText.text = "Connected: $serverUrl"
                startPolling()
            }
        }

        if (serverUrl.isNotEmpty()) {
            startPolling()
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        webView.settings.javaScriptEnabled = true
        webView.settings.domStorageEnabled = true
        webView.settings.allowFileAccess = true
        webView.settings.mediaPlaybackRequiresUserGesture = false

        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                return false
            }
        }

        webView.addJavascriptInterface(object {
            @JavascriptInterface
            fun getTypedNotes(): String {
                return ""
            }

            @JavascriptInterface
            fun onSubmitted() {
                runOnUiThread {
                    showSessionList()
                    startPolling()
                }
            }
        }, "Android")
    }

    private fun openSession(sessionId: String) {
        currentSessionId = sessionId
        pollJob?.cancel()
        sessionListContainer.visibility = View.GONE
        webView.visibility = View.VISIBLE
        webView.loadUrl("$serverUrl/session/$sessionId")
    }

    private fun showSessionList() {
        currentSessionId = null
        webView.visibility = View.GONE
        sessionListContainer.visibility = View.VISIBLE
    }

    private fun startPolling() {
        pollJob?.cancel()
        pollJob = scope.launch {
            while (isActive) {
                fetchSessions()
                delay(5000)
            }
        }
    }

    private suspend fun fetchSessions() {
        withContext(Dispatchers.IO) {
            try {
                val request = Request.Builder()
                    .url("$serverUrl/api/sessions")
                    .build()
                val response = client.newCall(request).execute()
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: "[]"
                    val arr = JSONArray(body)
                    val sessions = mutableListOf<SessionInfo>()
                    for (i in 0 until arr.length()) {
                        val obj = arr.getJSONObject(i)
                        sessions.add(SessionInfo(
                            id = obj.getString("id"),
                            title = obj.optString("title", "(untitled)"),
                            status = obj.getString("status"),
                            createdAt = obj.getString("created_at"),
                        ))
                    }
                    val hadSessions = adapter.itemCount
                    withContext(Dispatchers.Main) {
                        adapter.submitList(sessions)
                        // Vibrate on new session
                        if (sessions.size > hadSessions && hadSessions > 0) {
                            vibrate()
                        }
                    }
                }
            } catch (_: Exception) {
                // silently retry
            }
        }
    }

    private fun vibrate() {
        val vibrator = getSystemService(VIBRATOR_SERVICE) as? Vibrator
        vibrator?.vibrate(200)
    }

    @Deprecated("Use OnBackPressedCallback")
    override fun onBackPressed() {
        if (webView.visibility == View.VISIBLE) {
            showSessionList()
            startPolling()
        } else {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        scope.cancel()
    }
}

data class SessionInfo(
    val id: String,
    val title: String,
    val status: String,
    val createdAt: String,
)
